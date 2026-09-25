//! Optional project configuration, loaded from `kiln.json` or `kiln.toml`.
//!
//! Kiln is zero-config by default: every field here is optional and, when
//! unset, falls back to auto-detection. The file lets a project pin the pieces
//! detection cannot infer (a specific runtime version, a custom start command,
//! runtime env) without hand-writing a Dockerfile.

use std::collections::BTreeMap;
use std::path::Path;

use schemars::JsonSchema;
use serde::Deserialize;

use crate::detect::BuildOverrides;

/// User-provided build configuration. All fields are optional.
#[derive(Debug, Default, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KilnConfig {
    /// Force a provider instead of auto-detecting (e.g. `node`, `go`, `python`).
    pub provider: Option<String>,
    /// Runtime version for the base image, when the provider supports it
    /// (e.g. `22` or `22.5` for Node, `1.23` for Go, `3.12` for Python).
    pub version: Option<String>,
    /// Force a package manager (e.g. `npm`, `pnpm`, `yarn`, `bun`).
    pub package_manager: Option<String>,
    /// Replace the dependency-install command.
    pub install_command: Option<String>,
    /// Replace the build command (also forces a build stage when set).
    pub build_command: Option<String>,
    /// Replace the runtime start command.
    pub start_command: Option<String>,
    /// Port the application listens on.
    pub port: Option<u16>,
    /// Environment variables to set in the runtime image.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Apt packages to install in the build stage (compilers, headers, and other
    /// build-time system dependencies). Only takes effect on Debian-family build
    /// images (the default for every provider).
    #[serde(default)]
    pub build_apt_packages: Vec<String>,
    /// Apt packages to install in the final runtime image (shared libraries and
    /// other runtime system dependencies). Requires a Debian/Ubuntu-family runtime;
    /// on an apt-less base (distroless, Alpine, scratch) planning fails with a
    /// clear error rather than emitting a Dockerfile that breaks at build time.
    #[serde(default)]
    pub deploy_apt_packages: Vec<String>,
    /// Override the base image of the final runtime stage (e.g. to swap in a
    /// hardened or mirrored image).
    pub runtime_image: Option<String>,
    /// Override the base image of the first build stage.
    pub build_image: Option<String>,
    /// Directories to prepend to `PATH` in the runtime image.
    #[serde(default)]
    pub paths: Vec<String>,
    /// `BuildKit` secret ids to expose to build-stage commands (e.g. a private
    /// registry token). The value is supplied at build time and never written
    /// into an image layer. Each id is mounted at `/run/secrets/<id>`.
    #[serde(default)]
    pub secrets: Vec<String>,
    /// Commands to run at the start of the build stage, before the provider's own
    /// steps (e.g. generate code or fetch a private tool). Each becomes its own
    /// `RUN` line, in order.
    #[serde(default)]
    pub pre_build: Vec<String>,
    /// Commands to run at the end of the build stage, after the provider's own
    /// steps (e.g. a post-processing or asset step). Each becomes its own `RUN`
    /// line, in order.
    #[serde(default)]
    pub post_build: Vec<String>,
}

/// The JSON schema for [`KilnConfig`], pretty-printed. Feeds editor
/// autocompletion and validation for `kiln.json`.
#[must_use]
pub fn schema_json() -> String {
    let schema = schemars::schema_for!(KilnConfig);
    serde_json::to_string_pretty(&schema).unwrap_or_default()
}

impl KilnConfig {
    /// Load configuration from the project root, if present.
    ///
    /// Looks for `kiln.json` first, then `kiln.toml`. Returns `Ok(None)` when
    /// neither exists.
    ///
    /// # Errors
    ///
    /// Returns an error if a config file is present but cannot be parsed.
    pub fn load(root: impl AsRef<Path>) -> crate::error::Result<Option<Self>> {
        let root = root.as_ref();

        let json_path = root.join("kiln.json");
        if json_path.is_file() {
            let content = std::fs::read_to_string(&json_path).map_err(|source| crate::error::Error::ReadFile {
                path: json_path.clone(),
                source,
            })?;
            let config = serde_json::from_str(&content).map_err(|e| crate::error::Error::Parse {
                path: json_path,
                message: e.to_string(),
            })?;
            return Ok(Some(config));
        }

        let toml_path = root.join("kiln.toml");
        if toml_path.is_file() {
            let content = std::fs::read_to_string(&toml_path).map_err(|source| crate::error::Error::ReadFile {
                path: toml_path.clone(),
                source,
            })?;
            let config = toml::from_str(&content).map_err(|e| crate::error::Error::Parse {
                path: toml_path,
                message: e.to_string(),
            })?;
            return Ok(Some(config));
        }

        Ok(None)
    }

    /// Fold this configuration into `overrides`, filling only the fields that
    /// `overrides` leaves unset. Callers pass CLI-derived overrides so an
    /// explicit flag always wins over the config file.
    #[must_use]
    pub fn merge_into(self, mut overrides: BuildOverrides) -> BuildOverrides {
        overrides.provider = overrides.provider.or(self.provider);
        overrides.version = overrides.version.or(self.version);
        overrides.package_manager = overrides.package_manager.or(self.package_manager);
        overrides.install_command = overrides.install_command.or(self.install_command);
        overrides.build_command = overrides.build_command.or(self.build_command);
        overrides.start_command = overrides.start_command.or(self.start_command);
        overrides.port = overrides.port.or(self.port);
        if overrides.env.is_empty() {
            overrides.env = self.env;
        }
        if overrides.build_apt_packages.is_empty() {
            overrides.build_apt_packages = self.build_apt_packages;
        }
        if overrides.deploy_apt_packages.is_empty() {
            overrides.deploy_apt_packages = self.deploy_apt_packages;
        }
        overrides.runtime_image = overrides.runtime_image.or(self.runtime_image);
        overrides.build_image = overrides.build_image.or(self.build_image);
        if overrides.paths.is_empty() {
            overrides.paths = self.paths;
        }
        if overrides.secrets.is_empty() {
            overrides.secrets = self.secrets;
        }
        if overrides.pre_build.is_empty() {
            overrides.pre_build = self.pre_build;
        }
        if overrides.post_build.is_empty() {
            overrides.post_build = self.post_build;
        }
        overrides
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_none_without_a_config_file() {
        let dir = tempfile::tempdir().unwrap();
        assert!(KilnConfig::load(dir.path()).unwrap().is_none());
    }

    #[test]
    fn loads_kiln_json() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("kiln.json"),
            r#"{"provider":"node","version":"20","port":8080,"env":{"NODE_ENV":"production"}}"#,
        )
        .unwrap();
        let config = KilnConfig::load(dir.path()).unwrap().unwrap();
        assert_eq!(config.provider.as_deref(), Some("node"));
        assert_eq!(config.version.as_deref(), Some("20"));
        assert_eq!(config.port, Some(8080));
        assert_eq!(config.env.get("NODE_ENV").map(String::as_str), Some("production"));
    }

    #[test]
    fn loads_kiln_toml() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("kiln.toml"),
            "provider = \"go\"\nversion = \"1.23\"\nstart_command = \"/bin/app\"\n",
        )
        .unwrap();
        let config = KilnConfig::load(dir.path()).unwrap().unwrap();
        assert_eq!(config.provider.as_deref(), Some("go"));
        assert_eq!(config.version.as_deref(), Some("1.23"));
        assert_eq!(config.start_command.as_deref(), Some("/bin/app"));
    }

    #[test]
    fn json_takes_precedence_over_toml() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("kiln.json"), r#"{"provider":"node"}"#).unwrap();
        std::fs::write(dir.path().join("kiln.toml"), "provider = \"go\"\n").unwrap();
        let config = KilnConfig::load(dir.path()).unwrap().unwrap();
        assert_eq!(config.provider.as_deref(), Some("node"));
    }

    #[test]
    fn rejects_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("kiln.json"), r#"{"provderr":"node"}"#).unwrap();
        assert!(KilnConfig::load(dir.path()).is_err());
    }

    #[test]
    fn cli_overrides_win_over_config() {
        let config = KilnConfig {
            provider: Some("node".to_string()),
            start_command: Some("node file.js".to_string()),
            ..Default::default()
        };
        let cli = BuildOverrides {
            start_command: Some("node dist/main.js".to_string()),
            ..Default::default()
        };
        let merged = config.merge_into(cli);
        // CLI start command wins, config provider fills the gap
        assert_eq!(merged.start_command.as_deref(), Some("node dist/main.js"));
        assert_eq!(merged.provider.as_deref(), Some("node"));
    }
}
