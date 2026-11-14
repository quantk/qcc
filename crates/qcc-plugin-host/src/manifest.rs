use std::path::{Component, PathBuf};

use serde::{Deserialize, Serialize};

use crate::permissions::Permissions;

/// The plugin API version this host implements. Plugins state the version they were
/// built against; before 1.0 the versions must match exactly.
pub const API_VERSION: &str = "0.1";

/// File name of the manifest inside a plugin directory.
pub const MANIFEST_FILE: &str = "plugin.toml";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManifestError {
    #[error("invalid manifest: {0}")]
    Syntax(String),
    #[error("invalid plugin id {0:?}: use lowercase letters, digits and dashes")]
    InvalidId(String),
    #[error("plugin targets API {found}, this host implements {API_VERSION}")]
    ApiMismatch { found: String },
    #[error("path {0:?} must be relative and stay inside the plugin directory")]
    PathEscapes(PathBuf),
}

/// `plugin.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub version: String,
    /// Plugin API version the component was built against.
    pub api: String,
    /// The component, relative to the plugin directory.
    pub wasm: PathBuf,
    /// Optional web bundle, relative to the plugin directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web: Option<PathBuf>,
    /// Permissions the plugin requests.
    #[serde(default)]
    pub permissions: Permissions,
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        let manifest: Self =
            toml::from_str(text).map_err(|error| ManifestError::Syntax(error.to_string()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<(), ManifestError> {
        let id_ok = !self.id.is_empty()
            && self.id.len() <= 64
            && !self.id.starts_with('-')
            && self
                .id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !id_ok {
            return Err(ManifestError::InvalidId(self.id.clone()));
        }
        if self.api != API_VERSION {
            return Err(ManifestError::ApiMismatch {
                found: self.api.clone(),
            });
        }
        for path in std::iter::once(&self.wasm).chain(&self.web) {
            let inside = path.components().all(|c| matches!(c, Component::Normal(_)));
            if !inside || path.as_os_str().is_empty() {
                return Err(ManifestError::PathEscapes(path.clone()));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r#"
        id = "provider-echo"
        version = "0.1.0"
        api = "0.1"
        wasm = "plugin.wasm"
    "#;

    #[test]
    fn parses_permissions() {
        let manifest = Manifest::parse(&format!(
            "{BASE}\n[permissions]\ndeltas = true\nprocess = [\"claude\"]\n"
        ))
        .unwrap();
        assert_eq!(manifest.id, "provider-echo");
        assert!(manifest.permissions.deltas);
        assert!(!manifest.permissions.kv);
        assert!(manifest.permissions.allows_process("claude"));
        assert!(!manifest.permissions.allows_process("/usr/bin/claude"));
        assert_eq!(
            Manifest::parse(BASE).unwrap().permissions,
            Permissions::default()
        );
    }

    #[test]
    fn rejects_bad_manifests() {
        let with = |from: &str, to: &str| Manifest::parse(&BASE.replace(from, to));
        assert!(matches!(
            with("provider-echo", "Echo!"),
            Err(ManifestError::InvalidId(_))
        ));
        assert!(matches!(
            with("api = \"0.1\"", "api = \"9.0\""),
            Err(ManifestError::ApiMismatch { .. })
        ));
        assert!(matches!(
            with("plugin.wasm", "../evil.wasm"),
            Err(ManifestError::PathEscapes(_))
        ));
        assert!(matches!(
            with("plugin.wasm", "/abs.wasm"),
            Err(ManifestError::PathEscapes(_))
        ));
        assert!(matches!(
            Manifest::parse(&format!("{BASE}\nsurprise = 1\n")),
            Err(ManifestError::Syntax(_))
        ));
        assert!(matches!(
            Manifest::parse(&format!("{BASE}\n[permissions]\nnetwork = true\n")),
            Err(ManifestError::Syntax(_))
        ));
    }
}
