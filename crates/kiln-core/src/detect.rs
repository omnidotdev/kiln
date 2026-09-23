use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::plan::BuildPlan;
use crate::providers;

/// User-supplied overrides that take precedence over a provider's auto-detected
/// build steps, for setups auto-detect cannot handle (all optional; `None`
/// leaves the detected value in place).
#[derive(Debug, Default, Clone)]
pub struct BuildOverrides {
    /// Force a provider instead of auto-detecting (e.g. "node"|"go"|"python").
    pub provider: Option<String>,
    /// Runtime version for the base image, when the provider supports it.
    pub version: Option<String>,
    /// Force a package manager (e.g. "npm"|"pnpm"|"yarn"|"bun") instead of
    /// lockfile sniffing.
    pub package_manager: Option<String>,
    /// Replace the dependency-install command.
    pub install_command: Option<String>,
    /// Replace the build command (also forces a build stage when set).
    pub build_command: Option<String>,
    /// Replace the runtime start command.
    pub start_command: Option<String>,
    /// Override the port the application listens on.
    pub port: Option<u16>,
    /// Environment variables to set in the runtime image.
    pub env: std::collections::BTreeMap<String, String>,
}

/// Context for a project being analyzed.
#[derive(Debug)]
pub struct AppContext {
    /// Root path of the project
    pub root: PathBuf,
    /// User-supplied build-step overrides (empty by default).
    pub overrides: BuildOverrides,
}

impl AppContext {
    /// Create a new app context rooted at the given path.
    ///
    /// # Errors
    ///
    /// Returns an error if the path does not exist.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        Self::with_overrides(root, BuildOverrides::default())
    }

    /// Create a context with build-step overrides.
    ///
    /// # Errors
    ///
    /// Returns an error if the path does not exist.
    pub fn with_overrides(root: impl Into<PathBuf>, overrides: BuildOverrides) -> Result<Self> {
        let root = root.into();
        if !root.exists() {
            return Err(Error::ReadFile {
                path: root,
                source: std::io::Error::new(std::io::ErrorKind::NotFound, "path does not exist"),
            });
        }
        Ok(Self { root, overrides })
    }

    /// Check if a file exists relative to the project root.
    #[must_use]
    pub fn has_file(&self, name: &str) -> bool {
        self.root.join(name).is_file()
    }

    /// Read a file relative to the project root.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read.
    pub fn read_file(&self, name: &str) -> Result<String> {
        let path = self.root.join(name);
        std::fs::read_to_string(&path).map_err(|e| Error::ReadFile { path, source: e })
    }

    /// List files in a directory relative to the project root.
    #[must_use]
    pub fn list_files(&self, dir: &str) -> Vec<PathBuf> {
        let path = self.root.join(dir);
        std::fs::read_dir(&path)
            .ok()
            .map(|entries| entries.filter_map(std::result::Result::ok).map(|e| e.path()).collect())
            .unwrap_or_default()
    }

    /// Check if any file with the given extension exists in the root.
    #[must_use]
    pub fn has_file_with_extension(&self, ext: &str) -> bool {
        self.list_files(".")
            .iter()
            .any(|p| p.extension().is_some_and(|e| e == ext))
    }
}

/// Detect the project language and generate a build plan.
///
/// Tries each registered provider in priority order and returns
/// the plan from the first matching provider.
///
/// # Errors
///
/// Returns `NoProviderDetected` if no provider matches.
pub fn detect_and_plan(root: impl AsRef<Path>) -> Result<BuildPlan> {
    detect_and_plan_with(root, BuildOverrides::default())
}

/// Detect the project language and generate a build plan, applying
/// user-supplied build-step overrides.
///
/// # Errors
///
/// Returns `NoProviderDetected` if no provider matches.
pub fn detect_and_plan_with(root: impl AsRef<Path>, overrides: BuildOverrides) -> Result<BuildPlan> {
    let root = root.as_ref();

    // A kiln.json / kiln.toml fills any field the caller (CLI) left unset, so an
    // explicit flag always wins over the config file.
    let overrides = match crate::config::KilnConfig::load(root)? {
        Some(config) => config.merge_into(overrides),
        None => overrides,
    };

    let forced_provider = overrides.provider.clone();
    let port_override = overrides.port;
    let env = overrides.env.clone();
    let ctx = AppContext::with_overrides(root, overrides)?;

    let mut plan = if let Some(name) = forced_provider {
        let provider = providers::all()
            .into_iter()
            .find(|p| p.name() == name)
            .ok_or_else(|| Error::Provider(format!("unknown provider: {name}")))?;
        tracing::info!(provider = provider.name(), "using configured provider");
        provider.plan(&ctx)?
    } else {
        let mut plan = None;
        for provider in providers::all() {
            if provider.detect(&ctx) {
                tracing::info!(provider = provider.name(), "detected project language");
                plan = Some(provider.plan(&ctx)?);
                break;
            }
        }
        plan.ok_or_else(|| Error::NoProviderDetected(ctx.root.clone()))?
    };

    if let Some(port) = port_override {
        plan.port = Some(port);
    }
    plan.env = env;

    Ok(plan)
}

/// Detect the project language without generating a plan.
///
/// Returns the provider name if detected.
///
/// # Errors
///
/// Returns an error if the project root does not exist.
pub fn detect(root: impl AsRef<Path>) -> Result<Option<String>> {
    let ctx = AppContext::new(root.as_ref())?;

    for provider in providers::all() {
        if provider.detect(&ctx) {
            return Ok(Some(provider.name().to_string()));
        }
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_forces_provider_over_detection() {
        // a repo with BOTH go.mod and package.json; config forces node
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module example.com/app\n").unwrap();
        std::fs::write(dir.path().join("package.json"), r#"{"name":"t","main":"index.js"}"#).unwrap();
        std::fs::write(dir.path().join("kiln.json"), r#"{"provider":"node"}"#).unwrap();
        let plan = detect_and_plan(dir.path()).unwrap();
        assert_eq!(plan.provider, "node");
    }

    #[test]
    fn config_applies_port_and_env() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module example.com/app\n").unwrap();
        std::fs::write(
            dir.path().join("kiln.json"),
            r#"{"port":9000,"env":{"LOG_LEVEL":"debug"}}"#,
        )
        .unwrap();
        let plan = detect_and_plan(dir.path()).unwrap();
        assert_eq!(plan.port, Some(9000));
        assert_eq!(plan.env.get("LOG_LEVEL").map(String::as_str), Some("debug"));
    }

    #[test]
    fn unknown_configured_provider_errors() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module example.com/app\n").unwrap();
        std::fs::write(dir.path().join("kiln.json"), r#"{"provider":"cobol"}"#).unwrap();
        assert!(detect_and_plan(dir.path()).is_err());
    }
}
