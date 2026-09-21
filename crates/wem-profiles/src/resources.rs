//! Checksum-addressed package resources (Python: `profiles/resources.py`).
//!
//! Resource identity keys are *logical*: profiles-directory-relative POSIX
//! paths in canonical slash form on every platform. The platform path API
//! (`std::path`) is touched only at the final IO construction in
//! [`ResourceRef::read_bytes`]; key joining, parent extraction, and
//! comparison never round-trip through it, because its `display()` injects
//! the OS separator — which the shared key validator rejects as unsafe.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::data::DataDir;
use crate::error::ProfileError;

/// Validate one logical resource key (platform-independent string rules).
///
/// The single rejection rule shared by every entry point; rejects exactly
/// what Python `normalize_resource_path` rejects: empty text, backslashes,
/// absolute paths, and empty/`.`/`..` segments.
pub(crate) fn validate_logical_key(key: &str) -> Result<&str, ProfileError> {
    if key.is_empty()
        || key.contains('\\')
        || key.starts_with('/')
        || key
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(ProfileError::UnsafePath {
            path: key.to_string(),
        });
    }
    Ok(key)
}

/// Return a safe package-relative POSIX resource path.
///
/// Rejects exactly what Python `normalize_resource_path` rejects: empty
/// text, backslashes, absolute paths, and empty/`.`/`..` segments.
pub fn normalize_resource_path(path: &str) -> Result<PathBuf, ProfileError> {
    validate_logical_key(path).map(PathBuf::from)
}

/// The directory prefix of a logical resource key: everything before its
/// last '/', or the empty string when the key has no directory component.
///
/// Pure string logic — never touches `std::path`, so the result is
/// identical on every platform.
pub(crate) fn logical_parent(key: &str) -> &str {
    match key.rfind('/') {
        Some(pos) => &key[..pos],
        None => "",
    }
}

/// Join two logical resource keys with the canonical slash separator.
///
/// Pure string operation — never touches `std::path`, so the result is
/// identical on every platform (an OS separator cannot leak in).
pub(crate) fn join_logical_path(parent: &str, child: &str) -> String {
    if parent.is_empty() {
        child.to_string()
    } else {
        format!("{parent}/{child}")
    }
}

/// The manifest-directory-relative form of a logical resource key: the key
/// with a leading `parent/` prefix stripped, or the whole key when it does
/// not sit under `parent` (or `parent` is empty).
pub(crate) fn logical_relative<'a>(key: &'a str, parent: &str) -> &'a str {
    if parent.is_empty() {
        key
    } else if key
        .strip_prefix(parent)
        .is_some_and(|rest| rest.starts_with('/'))
    {
        &key[parent.len() + 1..]
    } else {
        key
    }
}

/// Where a resource's bytes come from.
///
/// * [`ResourceBackend::Fs`] — the standard filesystem backend: files live
///   under the profiles directory of a [`DataDir`] (Python's
///   `importlib.resources` path).
/// * [`ResourceBackend::Bytes`] — in-memory bytes, keyed by
///   profiles-directory-relative POSIX path. The threadless (e.g.
///   wasm32-unknown-unknown) entry point: no filesystem access, while the
///   SHA-256 / schema / path-safety validation is shared with the filesystem
///   path ([`ResourceRef::read_bytes`] runs the same digest check either way).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceBackend {
    /// Filesystem backend rooted at one profile data tree.
    Fs(DataDir),
    /// In-memory backend: profiles-dir-relative POSIX path -> file bytes.
    ///
    /// Duplicate paths collapse with last-one-wins, mirroring how the
    /// Python zip-based import resolves repeated member names.
    Bytes { files: BTreeMap<String, Vec<u8>> },
    /// Compile-time embedded profile bytes. They are copied only when a
    /// resource is decoded, not once for the whole bundle at startup.
    Static {
        files: &'static [(&'static str, &'static [u8])],
    },
}

/// Immutable package resource identity validated by SHA-256
/// (Python `ResourceRef`). Paths are profiles-directory-relative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceRef {
    backend: ResourceBackend,
    /// Canonical logical key: profiles-directory-relative POSIX path,
    /// slash-separated on every platform.
    path: String,
    sha256: String,
}

impl ResourceRef {
    /// Validate package/identity and normalize; mirrors Python `__post_init__`.
    pub fn new(data: DataDir, path: &str, sha256: &str) -> Result<Self, ProfileError> {
        Self::with_backend(ResourceBackend::Fs(data), path, sha256)
    }

    /// Construct against an explicit byte source (shared validator; used by
    /// the in-memory entry point). Same path / digest rejection rules as
    /// [`ResourceRef::new`].
    pub fn with_backend(
        backend: ResourceBackend,
        path: &str,
        sha256: &str,
    ) -> Result<Self, ProfileError> {
        let path = validate_logical_key(path)?.to_string();
        let digest = sha256.to_lowercase();
        if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ProfileError::InvalidSha256 {
                value: sha256.to_string(),
            });
        }
        Ok(Self {
            backend,
            path,
            sha256: digest,
        })
    }

    /// The byte source this reference resolves against.
    pub fn backend(&self) -> &ResourceBackend {
        &self.backend
    }

    /// The canonical logical key: profiles-directory-relative POSIX path,
    /// slash-separated on every platform (never a platform separator).
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Lowercase SHA-256 hex identity.
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    /// Read bytes, verifying SHA-256 (Python `read_bytes`).
    ///
    /// The read source is the [`ResourceBackend`]; the SHA-256 check below
    /// applies identically to both filesystem and in-memory sources, so the
    /// two entry points cannot drift in what they accept or reject.
    pub fn read_bytes(&self) -> Result<Vec<u8>, ProfileError> {
        let payload = match &self.backend {
            ResourceBackend::Fs(data) => {
                // Final IO construction: the only place the logical key
                // meets an OS path.
                let full = data.profiles_dir().join(&self.path);
                std::fs::read(&full).map_err(|_| ProfileError::MissingResource {
                    path: self.path.clone(),
                })?
            }
            ResourceBackend::Bytes { files } => {
                files
                    .get(&self.path)
                    .cloned()
                    .ok_or_else(|| ProfileError::MissingResource {
                        path: self.path.clone(),
                    })?
            }
            ResourceBackend::Static { files } => files
                .iter()
                .find_map(|(path, bytes)| (*path == self.path).then_some(*bytes))
                .map(Vec::from)
                .ok_or_else(|| ProfileError::MissingResource {
                    path: self.path.clone(),
                })?,
        };
        let mut hasher = Sha256::new();
        hasher.update(&payload);
        let actual = hex(hasher.finalize());
        if actual != self.sha256 {
            return Err(ProfileError::ShaMismatch {
                path: self.path.clone(),
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
            path: self.path.clone(),
        })
    }

    /// Read and parse JSON with verification (Python `read_json`).
    pub fn read_json(&self) -> Result<Value, ProfileError> {
        let raw = self.read_text()?;
        serde_json::from_str(&raw).map_err(|_| ProfileError::InvalidJson {
            path: self.path.clone(),
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

#[cfg(test)]
mod tests {
    use super::{join_logical_path, logical_parent, logical_relative, validate_logical_key};

    #[test]
    fn join_logical_path_stitches_with_slash_only() {
        assert_eq!(join_logical_path("a/b", "c/d"), "a/b/c/d");
        assert_eq!(join_logical_path("p", "x/y/z.json"), "p/x/y/z.json");
    }

    #[test]
    fn join_logical_path_with_empty_parent_is_the_child() {
        assert_eq!(join_logical_path("", "c/d"), "c/d");
    }

    #[test]
    fn logical_parent_reads_the_prefix_before_the_last_slash() {
        assert_eq!(logical_parent("a/b/manifest.json"), "a/b");
        assert_eq!(logical_parent("a/b"), "a");
        assert_eq!(logical_parent("manifest.json"), "");
        assert_eq!(logical_parent(""), "");
    }

    #[test]
    fn logical_relative_strips_only_an_exact_parent_prefix() {
        assert_eq!(logical_relative("a/b/c/d", "a/b"), "c/d");
        assert_eq!(logical_relative("c/d", "a/b"), "c/d");
        assert_eq!(logical_relative("a/bx/c/d", "a/b"), "a/bx/c/d");
        assert_eq!(logical_relative("a/b/c/d", ""), "a/b/c/d");
    }

    #[test]
    fn logical_key_validation_is_platform_independent() {
        assert!(validate_logical_key("a/b/c.d").is_ok());
        assert!(validate_logical_key("a\\b").is_err());
        assert!(validate_logical_key("/abs").is_err());
        assert!(validate_logical_key("a//b").is_err());
        assert!(validate_logical_key("a/./b").is_err());
        assert!(validate_logical_key("a/../b").is_err());
        assert!(validate_logical_key("").is_err());
    }
}
