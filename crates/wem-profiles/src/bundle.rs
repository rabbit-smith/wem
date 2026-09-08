//! Validated, path-safe encoder profile bundles
//! (Python: `profiles/bundle.py`).

use std::collections::BTreeMap;

use serde_json::Value;

use crate::data::DataDir;
use crate::error::ProfileError;
use crate::key::ProfileKey;
use crate::model::ContainerMetadata;
use crate::resources::{
    join_logical_path, logical_parent, logical_relative, normalize_resource_path,
    validate_logical_key, ResourceBackend, ResourceRef,
};

/// Profile index schema (Python `INDEX_SCHEMA`).
pub const INDEX_SCHEMA: &str = "wwise-wem.profile-index.v1";
/// Profile manifest schema (Python `BUNDLE_SCHEMA` / `MANIFEST_SCHEMA`).
pub const BUNDLE_SCHEMA: &str = "wwise-wem.profile-manifest.v1";

/// Profile manifest and its checksum-verified logical resources
/// (Python `RuntimeResourceManifest`).
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeResourceManifest {
    /// The manifest's own checksum-addressed reference.
    ref_: ResourceRef,
    schema: String,
    /// Logical name -> resource (insertion order of the JSON object).
    resources: Vec<(String, ResourceRef)>,
}

impl RuntimeResourceManifest {
    /// Load from an already-parsed manifest payload
    /// (Python `RuntimeResourceManifest.load`).
    pub(crate) fn load(ref_: &ResourceRef, payload: &Value) -> Result<Self, ProfileError> {
        let schema = payload
            .get("schema")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let resources_value = match payload.get("resources") {
            Some(Value::Object(map)) => map,
            _ => return Err(ProfileError::ManifestResourcesNotObject),
        };
        let mut resources = Vec::with_capacity(resources_value.len());
        // Python: resources resolve against `ref.path.parent` (the profile
        // directory), so resource entries are profile-dir-relative.
        // The parent comes from the ref key's string shape (the prefix
        // before its last '/'), never through std::path — whose display()
        // would inject the OS separator on Windows and the joined key would
        // fail the shared rejection rules below.
        let manifest_parent = logical_parent(ref_.path());
        for (name, entry) in resources_value {
            if name.is_empty() {
                return Err(ProfileError::ResourceNameEmpty);
            }
            let entry_map = entry
                .as_object()
                .ok_or_else(|| ProfileError::ResourceEntryNotObject { name: name.clone() })?;
            let path = entry_map
                .get("path")
                .and_then(Value::as_str)
                .filter(|p| !p.is_empty())
                .ok_or_else(|| ProfileError::ResourcePathMissing { name: name.clone() })?;
            let path = validate_logical_key(path)?;
            let full_path = join_logical_path(manifest_parent, path);
            let sha256 = entry_map
                .get("sha256")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| ProfileError::ResourceShaMissing { name: name.clone() })?;
            // The optional `schema` entry is carried by the payload but not
            // validated, exactly like the Python loader.
            // Child refs inherit the parent ref's byte source (filesystem or
            // in-memory) so both entry points validate identically.
            let ref_ = ResourceRef::with_backend(ref_.backend().clone(), &full_path, sha256)
                .map_err(|_| ProfileError::ResourcePathMissing { name: name.clone() })?;
            resources.push((name.clone(), ref_));
        }
        if schema != BUNDLE_SCHEMA {
            return Err(ProfileError::UnsupportedManifestSchema { schema });
        }
        if resources.is_empty() {
            return Err(ProfileError::ManifestResourcesEmpty);
        }
        Ok(Self {
            ref_: ref_.clone(),
            schema,
            resources,
        })
    }

    /// The manifest resource reference.
    pub fn resource_ref(&self) -> &ResourceRef {
        &self.ref_
    }

    /// Profiles-dir-relative manifest logical key (canonical slash form on
    /// every platform).
    pub fn ref_path(&self) -> &str {
        self.ref_.path()
    }

    pub fn schema(&self) -> &str {
        &self.schema
    }

    /// All logical resources in manifest order.
    pub fn resources(&self) -> &[(String, ResourceRef)] {
        &self.resources
    }

    /// Look up a resource by logical name, or by its manifest-relative path
    /// (Python `resource`).
    pub fn resource(&self, name_or_path: &str) -> Result<&ResourceRef, ProfileError> {
        if let Some((_, ref_)) = self.resources.iter().find(|(name, _)| name == name_or_path) {
            return Ok(ref_);
        }
        // Path-form lookup compares canonical logical keys (slash form on
        // every platform), manifest-parent-relative on both sides.
        let requested = logical_relative(
            validate_logical_key(name_or_path)?,
            logical_parent(self.ref_.path()),
        );
        for (_, ref_) in &self.resources {
            if logical_relative(ref_.path(), logical_parent(self.ref_.path())) == requested {
                return Ok(ref_);
            }
        }
        Err(ProfileError::ResourceNotFound {
            key: name_or_path.to_string(),
        })
    }

    /// Verify every resource in sorted name order (Python `verify_all`).
    pub fn verify_all(&self) -> Result<(), ProfileError> {
        let mut names: Vec<&String> = self.resources.iter().map(|(n, _)| n).collect();
        names.sort_unstable();
        for name in names {
            self.resources
                .iter()
                .find(|(n, _)| n == name)
                .expect("name came from the resource list")
                .1
                .verify()?;
        }
        Ok(())
    }
}

/// Complete immutable profile identity and its runtime resource set
/// (Python `ProfileBundle`).
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileBundle {
    name: String,
    key: ProfileKey,
    container_metadata: ContainerMetadata,
    runtime_manifest: RuntimeResourceManifest,
    block_sizes: [i64; 2],
    /// Whether the manifest carries the `vorbis.setup` resource.
    ///
    /// A bundle without setup is a draft profile: its static resources are
    /// checksummed and usable (e.g. the quality-curves mechanism), but no
    /// encode can run on it until a paired Wwise export supplies the setup
    /// packet. `setup_available == false` is reported by the registry
    /// together with [`ProfileBundle::pending_reason`].
    setup_available: bool,
    /// Manifest-declared reason the profile is not yet encodable
    /// (`None` when the profile is complete).
    pending_reason: Option<String>,
}

impl ProfileBundle {
    /// Validate and construct (Python `__post_init__` checks).
    pub(crate) fn new(
        name: String,
        key: ProfileKey,
        container_metadata: ContainerMetadata,
        runtime_manifest: RuntimeResourceManifest,
        block_sizes: [i64; 2],
        setup_available: bool,
        pending_reason: Option<String>,
    ) -> Result<Self, ProfileError> {
        if name.is_empty() {
            return Err(ProfileError::BundleNameEmpty);
        }
        if (key.channels(), key.sample_rate())
            != (
                container_metadata.n_channels,
                container_metadata.n_samples_per_sec,
            )
        {
            return Err(ProfileError::BundleGeometryMismatch);
        }
        let p0 = container_metadata.u_blocksize0_pow;
        let p1 = container_metadata.u_blocksize1_pow;
        if p0 >= 0 && p1 >= 0 {
            let expected = [
                1i64 << container_metadata.u_blocksize0_pow,
                1i64 << container_metadata.u_blocksize1_pow,
            ];
            if block_sizes != expected {
                return Err(ProfileError::BundleBlockSizesMismatch);
            }
        } else {
            return Err(ProfileError::BundleBlockSizesMismatch);
        }
        // A draft profile (setup pending) skips the setup identity check;
        // complete profiles keep the historical verification exactly.
        if setup_available {
            let setup_ref = runtime_manifest
                .resources()
                .iter()
                .find(|(n, _)| n == "vorbis.setup")
                .map(|(_, ref_)| ref_.clone());
            let Some(setup_ref) = setup_ref else {
                return Err(ProfileError::BundleMissingVorbisSetup);
            };
            let setup_sha = setup_ref.sha256().to_string();
            if key.quality_setup_identity() != Some(format!("sha256:{setup_sha}").as_str()) {
                return Err(ProfileError::BundleSetupIdentityMismatch);
            }
        }
        Ok(Self {
            name,
            key,
            container_metadata,
            runtime_manifest,
            block_sizes,
            setup_available,
            pending_reason,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn key(&self) -> &ProfileKey {
        &self.key
    }

    pub fn container_metadata(&self) -> &ContainerMetadata {
        &self.container_metadata
    }

    pub fn runtime_manifest(&self) -> &RuntimeResourceManifest {
        &self.runtime_manifest
    }

    pub fn block_sizes(&self) -> [i64; 2] {
        self.block_sizes
    }

    /// Whether the profile carries its `vorbis.setup` resource.
    pub fn setup_available(&self) -> bool {
        self.setup_available
    }

    /// The manifest-declared pending reason for a draft profile
    /// (`None` when the profile is complete).
    pub fn pending_reason(&self) -> Option<&str> {
        self.pending_reason.as_deref()
    }

    /// The `vorbis.setup` resource (Python `setup` property).
    pub fn setup(&self) -> Result<&ResourceRef, ProfileError> {
        self.runtime_manifest.resource("vorbis.setup")
    }

    /// Verified setup packet bytes.
    pub fn setup_packet(&self) -> Result<Vec<u8>, ProfileError> {
        self.setup()?.read_bytes()
    }

    /// Verify every logical resource.
    pub fn verify_all(&self) -> Result<(), ProfileError> {
        self.runtime_manifest.verify_all()
    }

    /// Convert to the public identity model
    /// (Python registry construction of `EncoderProfile`).
    pub fn to_encoder_profile(&self) -> Result<crate::model::EncoderProfile, ProfileError> {
        let setup_ref = match self.setup() {
            Ok(ref_) => Some(ref_.clone()),
            Err(_) => None, // draft profile: setup pending
        };
        let setup_sha = setup_ref.as_ref().map(|ref_| ref_.sha256().to_string()).unwrap_or_default();
        crate::model::EncoderProfile::new(
            self.name.clone(),
            self.key.clone(),
            setup_ref,
            setup_sha,
            None,
            self.block_sizes,
            self.container_metadata.clone(),
            self.setup_available,
            self.pending_reason.clone(),
        )
    }
}

/// Strict JSON integer (mirrors Python `isinstance(x, int) and not bool`).
fn strict_int(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .filter(|_| matches!(value, Value::Number(n) if n.is_i64() || n.is_u64()))
}

/// Strict JSON non-empty string (mirrors Python `_string`).
fn strict_string(value: &Value) -> Option<&str> {
    value.as_str().filter(|s| !s.is_empty())
}

/// Extract key / container_metadata / block_sizes from the manifest payload,
/// mirroring the Python try/except that wraps failures as
/// "profile manifest fields are malformed".
fn build_bundle_fields(
    payload: &serde_json::Map<String, Value>,
) -> Result<(ProfileKey, ContainerMetadata, [i64; 2]), ProfileError> {
    let result = (|| -> Result<(ProfileKey, ContainerMetadata, Vec<i64>), String> {
        let key_data = payload
            .get("key")
            .and_then(Value::as_object)
            .ok_or_else(|| "key must be an object".to_string())?;
        let channels = strict_int(
            key_data
                .get("channels")
                .ok_or_else(|| "'channels'".to_string())?,
        )
        .ok_or_else(|| "key.channels must be an integer".to_string())?;
        let sample_rate = strict_int(
            key_data
                .get("sample_rate")
                .ok_or_else(|| "'sample_rate'".to_string())?,
        )
        .ok_or_else(|| "key.sample_rate must be an integer".to_string())?;
        let generation = strict_string(
            key_data
                .get("generation")
                .ok_or_else(|| "'generation'".to_string())?,
        )
        .ok_or_else(|| "key.generation must be non-empty text".to_string())?
        .to_string();
        let channel_layout = strict_string(
            key_data
                .get("channel_layout")
                .ok_or_else(|| "'channel_layout'".to_string())?,
        )
        .ok_or_else(|| "key.channel_layout must be non-empty text".to_string())?
        .to_string();
        let quality_setup_identity = strict_string(
            key_data
                .get("quality_setup_identity")
                .ok_or_else(|| "'quality_setup_identity'".to_string())?,
        )
        .ok_or_else(|| "key.quality_setup_identity must be non-empty text".to_string())?
        .to_string();
        let key = ProfileKey::with_identity(
            channels,
            sample_rate,
            Some(generation),
            Some(channel_layout),
            Some(quality_setup_identity),
        )
        .map_err(|e| e.to_string())?;

        let metadata_map = payload
            .get("container_metadata")
            .and_then(Value::as_object)
            .ok_or_else(|| "container_metadata must be an object".to_string())?;
        let metadata = ContainerMetadata::from_fmt_map(metadata_map).map_err(|e| e.to_string())?;

        let block_data = payload
            .get("block_sizes")
            .ok_or_else(|| "'block_sizes'".to_string())?;
        if !block_data.is_array() {
            return Err("profile bundle block_sizes must be an array".to_string());
        }
        let blocks: Vec<i64> = block_data
            .as_array()
            .unwrap()
            .iter()
            .map(|v| {
                strict_int(v).ok_or_else(|| "block_sizes entry must be an integer".to_string())
            })
            .collect::<Result<_, _>>()?;
        Ok((key, metadata, blocks))
    })();
    match result {
        Ok((key, metadata, blocks)) => {
            // Python builds a tuple of arbitrary length here; ProfileBundle
            // rejects anything but exactly two entries.
            if blocks.len() != 2 {
                return Err(ProfileError::ManifestFieldMalformed {
                    reason: format!(
                        "profile bundle block_sizes must contain two entries, found {}",
                        blocks.len()
                    ),
                });
            }
            Ok((key, metadata, [blocks[0], blocks[1]]))
        }
        Err(reason) => Err(ProfileError::ManifestFieldMalformed { reason }),
    }
}

/// Load a profile through the package index and its self-contained manifest
/// (Python `load_profile_bundle`).
///
/// * `profile` selects an index entry; `None` uses the index `default`.
/// * `verify_all` runs SHA-256 verification over every logical resource.
pub fn load_profile_bundle(
    data: &DataDir,
    profile: Option<&str>,
    verify_all: bool,
) -> Result<ProfileBundle, ProfileError> {
    let raw = std::fs::read(data.index_path()).map_err(|_| ProfileError::MissingResource {
        path: "index.json".to_string(),
    })?;
    load_profile_bundle_with(&raw, ResourceBackend::Fs(data.clone()), profile, verify_all)
}

/// Load a profile bundle entirely from in-memory bytes — no filesystem.
///
/// The threadless (e.g. wasm32-unknown-unknown) entry point that mirrors
/// [`load_profile_bundle`]:
///
/// * `index` is the raw bytes of one profile-index document (the same
///   `index.json` the filesystem entry reads).
/// * `files` maps every profiles-dir-relative POSIX resource path (the
///   manifest document plus each logical resource, e.g.
///   `wwise2013-6ch-44100/vorbis/setup.bin`) to its bytes, exactly as the
///   index/manifest entries name them. Duplicate paths: last wins.
/// * `profile` selects an index entry; `None` uses the index `default`.
/// * `verify_all` runs SHA-256 verification over every logical resource.
///
/// Both entry points funnel through the same core ([`load_profile_bundle_with`]),
/// so schema checks, SHA-256 identity checks, and path-safety rejections
/// apply identically — rejection conditions cannot drift between the two.
pub fn load_profile_bundle_from_bytes(
    index: &[u8],
    files: impl IntoIterator<Item = (String, Vec<u8>)>,
    profile: Option<&str>,
    verify_all: bool,
) -> Result<ProfileBundle, ProfileError> {
    let mut map = BTreeMap::new();
    for (path, bytes) in files {
        map.insert(path, bytes);
    }
    load_profile_bundle_with(
        index,
        ResourceBackend::Bytes { files: map },
        profile,
        verify_all,
    )
}

/// Shared index -> manifest -> bundle core; both entry points funnel here.
///
/// * `profile` selects an index entry; `None` uses the index `default`.
/// * `verify_all` runs SHA-256 verification over every logical resource.
fn load_profile_bundle_with(
    index_bytes: &[u8],
    backend: ResourceBackend,
    profile: Option<&str>,
    verify_all: bool,
) -> Result<ProfileBundle, ProfileError> {
    let index: Value =
        serde_json::from_slice(index_bytes).map_err(|_| ProfileError::InvalidJson {
            path: "index.json".to_string(),
        })?;
    let index = match index {
        Value::Object(map) => map,
        _ => {
            return Err(ProfileError::IndexNotObject {
                path: "index.json".to_string(),
            })
        }
    };

    let schema = index.get("schema").and_then(Value::as_str).unwrap_or("");
    if schema != INDEX_SCHEMA {
        return Err(ProfileError::UnsupportedIndexSchema {
            schema: schema.to_string(),
        });
    }

    let selected = match profile {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => index
            .get("default")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or(ProfileError::IndexDefaultMissing)?
            .to_string(),
    };

    let profiles = index
        .get("profiles")
        .and_then(Value::as_object)
        .ok_or(ProfileError::IndexProfilesNotObject)?;
    let index_entry = profiles
        .get(&selected)
        .ok_or_else(|| ProfileError::ProfileNotInIndex {
            profile: selected.clone(),
        })?;
    let index_entry =
        index_entry
            .as_object()
            .ok_or_else(|| ProfileError::IndexProfileNotObject {
                profile: selected.clone(),
            })?;

    let manifest_relative = strict_string(index_entry.get("manifest").unwrap_or(&Value::Null))
        .ok_or(ProfileError::IndexManifestPathMissing)?;
    let _ = normalize_resource_path(manifest_relative)?;
    let sha256 = strict_string(index_entry.get("sha256").unwrap_or(&Value::Null))
        .ok_or(ProfileError::IndexProfileShaMissing)?;

    let manifest_ref = ResourceRef::with_backend(backend, manifest_relative, sha256)?;
    let payload_raw = manifest_ref.read_bytes()?;
    let payload: Value =
        serde_json::from_slice(&payload_raw).map_err(|_| ProfileError::InvalidJson {
            path: manifest_ref.path().to_string(),
        })?;
    let payload = match payload {
        Value::Object(map) => map,
        _ => {
            return Err(ProfileError::ManifestNotObject {
                path: manifest_ref.path().to_string(),
            })
        }
    };

    let schema = payload.get("schema").and_then(Value::as_str).unwrap_or("");
    if schema != BUNDLE_SCHEMA {
        return Err(ProfileError::UnsupportedManifestSchema {
            schema: schema.to_string(),
        });
    }

    let (key, container_metadata, block_sizes) = build_bundle_fields(&payload)?;
    let payload_value = Value::Object(payload.clone());
    let runtime_manifest = RuntimeResourceManifest::load(&manifest_ref, &payload_value)?;

    let name = strict_string(payload.get("name").unwrap_or(&Value::Null))
        .ok_or(ProfileError::BundleNameEmpty)?
        .to_string();

    // Draft-profile metadata (additive; absent on complete profiles):
    // a profile whose manifest lacks the vorbis.setup resource is a
    // pending-corpus draft and is surfaced by the registry as such.
    let setup_available = payload
        .get("resources")
        .and_then(Value::as_object)
        .is_some_and(|resources| resources.contains_key("vorbis.setup"));
    let pending_reason = strict_string(payload.get("pending_reason").unwrap_or(&Value::Null))
        .map(str::to_string);

    let bundle = ProfileBundle::new(
        name,
        key,
        container_metadata,
        runtime_manifest,
        block_sizes,
        setup_available,
        pending_reason,
    )?;
    if bundle.name() != selected {
        return Err(ProfileError::IndexNameMismatch {
            selected,
            name: bundle.name().to_string(),
        });
    }
    if verify_all {
        bundle.verify_all()?;
    }
    Ok(bundle)
}
